#!/usr/bin/env python3
"""Generates the TypeScript client from the committed OpenAPI document.

There is exactly one description of this API — `docs/api/openapi.json`, itself generated
from the Rust contracts — and this script turns it into types and a client. Nothing here is
hand-maintained, so the frontend cannot drift from the backend by editing a DTO.

    python tools/api-client/generate.py            # write the client
    python tools/api-client/generate.py --check    # fail if the committed client is stale
"""
from __future__ import annotations

import argparse
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
SPEC = ROOT / "docs" / "api" / "openapi.json"
OUT = ROOT / "frontend" / "api-client" / "src"

HEADER = """// Generated from docs/api/openapi.json by tools/api-client/generate.py.
// Do not edit: run `python tools/api-client/generate.py` after changing the Rust contracts.
"""


def ts_name(ref: str) -> str:
    return ref.rsplit("/", 1)[-1]


def type_of(schema: dict, required: bool = True) -> str:
    """Renders one JSON Schema node as a TypeScript type."""
    if "$ref" in schema:
        rendered = ts_name(schema["$ref"])
    elif "oneOf" in schema:
        rendered = " | ".join(type_of(part) for part in schema["oneOf"])
    elif "allOf" in schema:
        rendered = " & ".join(type_of(part) for part in schema["allOf"])
    elif "enum" in schema:
        rendered = " | ".join(json.dumps(value) for value in schema["enum"])
    else:
        kind = schema.get("type")
        if isinstance(kind, list):
            parts = [type_of({**schema, "type": one}) for one in kind if one != "null"]
            rendered = " | ".join(parts) or "unknown"
            if "null" in kind:
                rendered += " | null"
        elif kind == "array":
            rendered = f"Array<{type_of(schema.get('items', {}))}>"
        elif kind == "object":
            rendered = render_object(schema)
        elif kind == "string":
            rendered = "string"
        elif kind in ("integer", "number"):
            # Only non-monetary counters reach this branch: money is `Decimal`, a string.
            rendered = "number"
        elif kind == "boolean":
            rendered = "boolean"
        elif kind == "null":
            rendered = "null"
        else:
            rendered = "unknown"
    if not required:
        rendered += " | undefined"
    return rendered


def render_object(schema: dict) -> str:
    properties = schema.get("properties") or {}
    if not properties:
        extra = schema.get("additionalProperties")
        if isinstance(extra, dict):
            return f"Record<string, {type_of(extra)}>"
        return "Record<string, unknown>"
    required = set(schema.get("required") or [])
    fields = []
    for name, prop in properties.items():
        optional = "" if name in required else "?"
        doc = prop.get("description")
        if doc:
            fields.append("  /** %s */" % " ".join(doc.split()))
        fields.append(f"  {name}{optional}: {type_of(prop)};")
    return "{\n" + "\n".join(fields) + "\n}"


def render_types(spec: dict) -> str:
    lines = [HEADER, ""]
    for name, schema in sorted(spec["components"]["schemas"].items()):
        doc = schema.get("description")
        if doc:
            lines.append("/** %s */" % " ".join(doc.split()))
        if "enum" in schema and schema.get("type") == "string":
            values = " | ".join(json.dumps(v) for v in schema["enum"])
            lines.append(f"export type {name} = {values};")
        else:
            lines.append(f"export type {name} = {type_of(schema)};")
        lines.append("")
    return "\n".join(lines)


def operation_name(method: str, path: str) -> str:
    parts = [p for p in re.split(r"[/{}-]", path.replace("/api/v1", "")) if p]
    camel = parts[0] + "".join(p.capitalize() for p in parts[1:])
    return camel if method == "get" else method + camel[0].upper() + camel[1:]


def render_client(spec: dict) -> str:
    lines = [
        HEADER,
        'import type * as T from "./types";',
        "",
        "/** Everything this client can fail with, as the server described it. */",
        "export class VoxApiError extends Error {",
        "  readonly status: number;",
        "  readonly body: T.ApiError;",
        "  constructor(status: number, body: T.ApiError) {",
        "    super(body.message);",
        "    this.status = status;",
        "    this.body = body;",
        "    this.name = \"VoxApiError\";",
        "  }",
        "}",
        "",
        "export interface VoxClientOptions {",
        "  /** Base URL of the Vox API. Same-origin by default. */",
        "  baseUrl?: string;",
        "  /** Passed through to fetch, for credentials and abort signals. */",
        "  fetch?: typeof fetch;",
        "  /** Cookie policy. Same-origin sends server-issued HttpOnly session cookie. */",
        "  credentials?: RequestCredentials;",
        "  /** Restored CSRF state from an earlier session bootstrap response. */",
        "  csrfToken?: string;",
        "}",
        "",
        "/** The generated transport client. Wrap it in a service; do not call fetch directly. */",
        "export class VoxClient {",
        "  private readonly baseUrl: string;",
        "  private readonly doFetch: typeof fetch;",
        "  private readonly credentials: RequestCredentials;",
        "  private csrfToken: string | undefined;",
        "",
        "  constructor(options: VoxClientOptions = {}) {",
        "    this.baseUrl = options.baseUrl ?? \"\";",
        "    this.doFetch = options.fetch ?? fetch;",
        "    this.credentials = options.credentials ?? \"same-origin\";",
        "    this.csrfToken = options.csrfToken;",
        "  }",
        "",
        "  private async request<R>(method: string, path: string, query?: Record<string, unknown>, body?: unknown): Promise<R> {",
        "    let resolvedPath = path;",
        "    const remainingQuery: Record<string, unknown> = {};",
        "    for (const [key, value] of Object.entries(query ?? {})) {",
        "      if (value === undefined || value === null) continue;",
        "      const marker = `{${key}}`;",
        "      if (resolvedPath.includes(marker)) resolvedPath = resolvedPath.replaceAll(marker, encodeURIComponent(String(value)));",
        "      else remainingQuery[key] = value;",
        "    }",
        "    const url = new URL(this.baseUrl + resolvedPath, this.baseUrl || \"http://localhost\");",
        "    for (const [key, value] of Object.entries(remainingQuery)) url.searchParams.set(key, String(value));",
        "    const headers: Record<string, string> = {};",
        "    if (![\"GET\", \"HEAD\", \"OPTIONS\"].includes(method) && this.csrfToken) headers[\"x-vox-csrf\"] = this.csrfToken;",
        "    const init: RequestInit = { method, credentials: this.credentials, headers };",
        "    if (body !== undefined) {",
        "      headers[\"content-type\"] = \"application/json\";",
        "      init.body = JSON.stringify(body);",
        "    }",
        "    const target = this.baseUrl ? url.toString() : url.pathname + url.search;",
        "    const response = await this.doFetch(target, init);",
        "    const payload = response.status === 204 ? undefined : await response.json();",
        "    if (!response.ok) throw new VoxApiError(response.status, payload as T.ApiError);",
        "    if (path === \"/api/v1/auth/session\") this.csrfToken = (payload as T.AuthSessionDto).csrf_token;",
        "    return payload as R;",
        "  }",
        "",
    ]
    for path, methods in sorted(spec["paths"].items()):
        for method, operation in sorted(methods.items()):
            if method not in ("get", "post", "put", "delete", "patch"):
                continue
            name = operation_name(method, path)
            summary = " ".join((operation.get("description") or operation.get("summary") or "").split())
            params = operation.get("parameters") or []
            query_type = "Record<string, unknown>" if params else None
            body_ref = None
            request_body = operation.get("requestBody")
            if request_body:
                content = request_body.get("content", {}).get("application/json", {})
                if "$ref" in content.get("schema", {}):
                    body_ref = ts_name(content["schema"]["$ref"])
            responses = operation.get("responses", {})
            success_codes = sorted(
                code for code in responses if code.isdigit() and 200 <= int(code) < 300
            )
            ok = responses.get(success_codes[0], {}) if success_codes else {}
            ret = "void"
            schema = ok.get("content", {}).get("application/json", {}).get("schema")
            if schema:
                ret = type_of(schema)
            args = []
            if query_type:
                rendered_params = sorted(
                    p["name"]
                    + ("" if p.get("required") else "?")
                    + ": "
                    + prefix_types(type_of(p.get("schema", {})))
                    for p in params
                )
                args.append("query: { %s }" % "; ".join(rendered_params))
            if body_ref:
                args.append(f"body: T.{body_ref}")
            call_args = ["\"%s\"" % method.upper(), "\"%s\"" % path]
            call_args.append("query" if query_type else "undefined")
            if body_ref:
                call_args.append("body")
            if summary:
                lines.append("  /** %s */" % summary)
            lines.append(
                "  %s(%s): Promise<%s> {" % (name, ", ".join(args), ret if ret == "void" else prefix_types(ret))
            )
            lines.append("    return this.request(%s);" % ", ".join(call_args))
            lines.append("  }")
            lines.append("")
    lines.append("}")
    lines.append("")
    return "\n".join(lines)


BUILTINS = {"Array", "Record", "Promise", "Date", "Map", "Set", "Partial", "Readonly"}


def prefix_types(rendered: str) -> str:
    """Qualifies generated type names with the `T.` import, leaving TypeScript built-ins alone."""
    return re.sub(
        r"\b([A-Z][A-Za-z0-9_]*)\b",
        lambda m: m.group(1) if m.group(1) in BUILTINS else "T." + m.group(1),
        rendered,
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true", help="fail if the committed client is stale")
    args = parser.parse_args()

    spec = json.loads(SPEC.read_text(encoding="utf-8"))
    files = {OUT / "types.ts": render_types(spec), OUT / "client.ts": render_client(spec)}

    if args.check:
        stale = []
        for path, content in files.items():
            current = path.read_text(encoding="utf-8") if path.is_file() else ""
            if current != content:
                stale.append(path.relative_to(ROOT).as_posix())
        if stale:
            print("generated client is stale: %s" % ", ".join(stale), file=sys.stderr)
            print("run: python tools/api-client/generate.py", file=sys.stderr)
            return 1
        print("generated client is current")
        return 0

    OUT.mkdir(parents=True, exist_ok=True)
    for path, content in files.items():
        path.write_text(content, encoding="utf-8")
        print("wrote %s" % path.relative_to(ROOT).as_posix())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
