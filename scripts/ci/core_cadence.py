"""Find the original full Core validation for a scheduled main revision."""

import json
import os
import re
from urllib.parse import urlencode
from urllib.request import Request, urlopen


CORE_JOB = re.compile(
    r"protocol|Rust model unit tests|Rust Responses unit tests|"
    r"Rust schema and identity tests|Rust targets \(integration-[1-4]\)"
)


def core_attempt(job):
    if not CORE_JOB.fullmatch(job["name"].rsplit(" / ", 1)[-1]):
        return False
    if job["status"] == "in_progress":
        return True
    if job["status"] != "completed" or job["conclusion"] in (None, "skipped", "neutral"):
        return False
    if job["conclusion"] in ("cancelled", "stale"):
        return any(
            step.get("started_at") and step.get("conclusion") != "skipped"
            for step in job.get("steps") or []
        )
    return True


def pages(fetch, path, field, **filters):
    page = 1
    while True:
        data = fetch(path, dict(filters, page=page, per_page=100))
        rows = data[field]
        yield from rows
        if len(rows) < 100 or page * 100 >= data["total_count"]:
            return
        if field == "workflow_runs" and page == 10:
            raise RuntimeError("Workflow history exceeds GitHub's search limit")
        page += 1


def prior_run(fetch, repo, sha, current):
    base = f"/repos/{repo}/actions"
    runs = pages(fetch, base + "/workflows/core-protocol.yml/runs", "workflow_runs",
                 branch="main", head_sha=sha)
    for run in runs:
        if run["id"] == current or run["head_sha"] != sha or run["head_branch"] != "main":
            continue
        head_repo = run.get("head_repository")
        if not head_repo:
            raise RuntimeError(f"Cannot verify repository for run {run['id']}")
        if head_repo["full_name"].lower() != repo.lower():
            continue
        if run["status"] not in ("in_progress", "completed"):
            continue
        # Browser-only and deduplicated runs may both have a successful conclusion.
        jobs = pages(fetch, f"{base}/runs/{run['id']}/jobs", "jobs", filter="latest")
        if any(core_attempt(job) for job in jobs):
            return run
    return None


def main():
    env = os.environ

    def fetch(path, query):
        url = env["GITHUB_API_URL"].rstrip("/") + path + "?" + urlencode(query)
        request = Request(url, headers={
            "Authorization": "Bearer " + env["GH_TOKEN"],
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
        })
        with urlopen(request, timeout=30) as response:
            return json.load(response)

    sha = env["GITHUB_SHA"]
    run = prior_run(fetch, env["GITHUB_REPOSITORY"], sha, int(env["GITHUB_RUN_ID"]))
    state = (run["conclusion"] or run["status"]) if run else "unseen"
    with open(env["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
        print("needed=" + str(run is None).lower(), file=output)
        print("prior_state=" + state, file=output)
    with open(env["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as summary:
        if run:
            print(f"Core `{sha}` already has [run {run['id']}]({run['html_url']}) (`{state}`).", file=summary)
        else:
            print(f"No earlier full Core attempt for `{sha}`. Starting qualification.", file=summary)


if __name__ == "__main__":
    main()
