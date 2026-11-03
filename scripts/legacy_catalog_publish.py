#!/usr/bin/env python3
"""Publish legacy model catalog metadata through Core channel control.

Input is catalog-metadata.json from `legacy_extensions_import.py --phase
catalog`. Core keeps vendors and per-route model metadata in the channel
release bindings, so this needs an active routing release (normally the first
one from legacy_routing_migration). Each legacy model is attached to every
route whose model name is exactly equal; models without such a route are only
reported, and a later run (after their routes exist) adds them.

Uses the supported management API with a superadmin browser Session:
GET /core/v1/admin/routing/configuration, then POST .../configuration/preview
with routing.version+1 and unchanged prices, then (with --publish) PUT
.../configuration with the preview's confirmation_digest. One atomic
publication; prices and routes are not changed. Existing different metadata
for the same vendor/route is a conflict, never overwritten. Dry-run (preview)
is the default.
"""
from __future__ import annotations

import argparse
import json
import urllib.error
import urllib.request
from pathlib import Path

PATH = "/core/v1/admin/routing/configuration"


def fail(code: str) -> None:
    raise SystemExit(f"error: {code}")


def call(args, method: str, path: str, body=None) -> dict:
    request = urllib.request.Request(
        args.core_url.rstrip("/") + path, method=method,
        data=None if body is None else json.dumps(body, ensure_ascii=False).encode(),
        headers={"Authorization": "Bearer " + args.secret, "Content-Type": "application/json"})
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    try:
        with opener.open(request, timeout=30) as response:
            value = json.load(response)
    except urllib.error.HTTPError as error:
        fail(f"http_{error.code}:{method}:{path}:{error.read()[:500].decode(errors='replace')}")
    if value.get("success") is not True:
        fail(f"core_rejected:{method}:{path}")
    return value["data"]


def merge(metadata: dict, current: dict) -> tuple[dict, dict]:
    bindings = current["bindings"]
    vendors = bindings.setdefault("vendors", {})
    models = bindings.setdefault("models", {})
    report = {"vendors_added": 0, "vendors_unchanged": 0, "routes_added": 0, "routes_unchanged": 0,
              "unrouted_models": []}
    for v in metadata["vendors"]:
        want = {"id": v["id"], "name": v["name"], "description": v["description"]}
        have = vendors.get(str(v["id"]))
        if have is not None and have != want:
            fail(f"vendor_conflict:{v['id']}")
        report["vendors_unchanged" if have else "vendors_added"] += 1
        vendors[str(v["id"])] = want
    routes: dict[str, list[int]] = {}
    for route in current["routing"]["routes"]:
        routes.setdefault(route["model"], []).append(route["id"])
    for m in metadata["models"]:
        want = {"display_name": m["display_name"], "description": m["description"], "vendor_id": m["vendor_id"]}
        ids = routes.get(m["model_name"], [])
        if not ids:
            report["unrouted_models"].append(m["model_name"])
        for route_id in ids:
            have = models.get(str(route_id))
            if have is not None and have != want:
                fail(f"model_metadata_conflict:{route_id}")
            report["routes_unchanged" if have else "routes_added"] += 1
            models[str(route_id)] = want
    return bindings, report


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--core-url", required=True, help="Core base URL, e.g. http://127.0.0.1:18171")
    p.add_argument("--session-file", required=True, type=Path, help="superadmin Session secret (0600 file)")
    p.add_argument("--metadata", required=True, type=Path, help="catalog-metadata.json")
    p.add_argument("--publish", action="store_true", help="publish after preview (default: preview only)")
    args = p.parse_args()
    if args.session_file.stat().st_mode & 0o077:
        fail("session_file_not_private")
    args.secret = args.session_file.read_text().strip()
    metadata = json.loads(args.metadata.read_text())
    if metadata.get("format_version") != 1 or metadata.get("scope") != "legacy-model-catalog-metadata":
        fail("metadata_scope_invalid")
    current = call(args, "GET", PATH)
    version = current["version"]
    bindings, report = merge(metadata, current)
    report["expected_version"] = version
    if not report["vendors_added"] and not report["routes_added"]:
        print(json.dumps({**report, "status": "unchanged"}, ensure_ascii=False, indent=1))
        return
    routing = {**current["routing"], "version": version["routing"] + 1}
    body = {"expected_version": version, "routing": routing, "prices": current["prices"], "bindings": bindings}
    preview = call(args, "POST", PATH + "/preview", body)
    report["confirmation_digest"] = preview["confirmation_digest"]
    if not args.publish:
        print(json.dumps({**report, "status": "previewed"}, ensure_ascii=False, indent=1))
        return
    published = call(args, "PUT", PATH, {**body, "confirmation_digest": preview["confirmation_digest"]})
    after = call(args, "GET", PATH)
    if after["version"] != published["version"] or after["bindings"].get("vendors") != bindings["vendors"] \
            or after["bindings"].get("models") != bindings["models"]:
        fail("readback_mismatch")
    print(json.dumps({**report, "status": "published", "version": after["version"]}, ensure_ascii=False, indent=1))


if __name__ == "__main__":
    main()
