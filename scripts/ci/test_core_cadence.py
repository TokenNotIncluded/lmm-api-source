import unittest

from core_cadence import core_attempt, prior_run


REPO = "example/lmm"
SHA = "a" * 40


def run(number, status="completed", result="success", **overrides):
    return {
        "id": number, "status": status, "conclusion": result, "head_sha": SHA,
        "head_branch": "main", "head_repository": {"full_name": REPO},
        "html_url": f"https://github.com/{REPO}/actions/runs/{number}", **overrides,
    }


def job(name="protocol", status="completed", result="success", steps=None):
    return {"name": name, "status": status, "conclusion": result, "steps": steps or []}


def history(runs, jobs):
    def fetch(path, query):
        if path.endswith("/workflows/core-protocol.yml/runs"):
            assert query["branch"] == "main" and query["head_sha"] == SHA
            field, rows = "workflow_runs", runs
        else:
            assert path.endswith("/jobs") and query["filter"] == "latest"
            field, rows = "jobs", jobs[int(path.split("/")[-2])]
        start = (query["page"] - 1) * query["per_page"]
        return {field: rows[start:start + query["per_page"]], "total_count": len(rows)}
    return fetch


class CoreCadence(unittest.TestCase):
    def test_browser_noop_current_and_other_sources_do_not_mask_failure(self):
        records = [
            run(20), run(19, head_branch="feature"), run(18, head_sha="b" * 40),
            run(17, head_repository={"full_name": "fork/lmm"}), run(16), run(15),
            run(14, "queued", None), run(13, result="failure"),
        ]
        jobs = {
            16: [job("Browser onboarding and same-request wallet receipt"), job(result="skipped")],
            15: [job("Select full verification"), job(result="skipped")],
            13: [job(result="failure")],
        }
        self.assertEqual(prior_run(history(records, jobs), REPO, SHA, 20), records[-1])

    def test_pending_and_cancelled_before_start_do_not_suppress_work(self):
        records = [run(9, "pending", None), run(8, "in_progress", None), run(7, result="cancelled")]
        jobs = {
            8: [job("Select full verification", "in_progress", None), job(status="queued", result=None)],
            7: [job(result="cancelled", steps=[{"conclusion": "skipped", "started_at": None}])],
        }
        self.assertIsNone(prior_run(history(records, jobs), REPO, SHA, 10))

    def test_running_success_and_started_interruption_are_receipts(self):
        for sample in [
            job("Rust targets (integration-2)", "in_progress", None),
            job("full / Rust Responses unit tests"),
            job(result="timed_out"),
            job(result="cancelled", steps=[{"started_at": "2026-11-27T00:02:00Z", "conclusion": "success"}]),
        ]:
            with self.subTest(job=sample):
                self.assertTrue(core_attempt(sample))

    def test_traverses_runs_and_jobs_pages(self):
        records = [run(i, "queued", None) for i in range(110, 10, -1)] + [run(10)]
        jobs = {10: [job("Unrelated") for _ in range(100)] + [job("Rust model unit tests")]}
        self.assertEqual(prior_run(history(records, jobs), REPO, SHA, 120)["id"], 10)

    def test_unverifiable_source_or_history_is_an_error(self):
        with self.assertRaises(RuntimeError):
            prior_run(history([run(3, head_repository=None)], {}), REPO, SHA, 4)

        def denied(path, query):
            raise PermissionError("actions:read unavailable")

        with self.assertRaises(PermissionError):
            prior_run(denied, REPO, SHA, 1)


if __name__ == "__main__":
    unittest.main()
