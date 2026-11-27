"""Offline regression coverage for read-only public release acceptance."""
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import sys
import unittest

spec = importlib.util.spec_from_file_location("public_production", Path(__file__).with_name("verify-public-production.py"))
module = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = module
spec.loader.exec_module(module)


class AcceptanceTest(unittest.TestCase):
    def setUp(self):
        self.responses = {
            "/health/ready": module.Response(200, "application/json", b'{"status":"ready","database":true}'),
            "/api/status": module.Response(200, "application/json", json.dumps({"success": True, "data": {"version": "0.2.51"}}).encode(), {"X-Request-Id": "fixture-request"}),
            "/app.js": module.Response(200, "application/javascript", b"console.log('loaded')"),
            "/app.css": module.Response(200, "text/css", b"body{margin:0}"),
        }
        for path in ("/", "/sign-in", "/console"):
            self.responses[path] = module.Response(200, "text/html", b'<script src="/app.js"></script><link rel="stylesheet" href="/app.css">')
        for locale, prefix in module.LEGAL_LOCALES.items():
            for doc in module.LEGAL_DOCUMENTS:
                self.responses[f"{prefix}/{doc}"] = module.Response(
                    200, "text/html",
                    f'<html lang="{locale}"><article>Contact support@lmm.best</article></html>'.encode(),
                )
                self.responses[f"/api/{doc}?lang={locale}"] = module.Response(
                    200, "application/json",
                    json.dumps({"success": True, "language": locale.lower(), "data": "Contact support@lmm.best"}).encode(),
                )
        self.visited = []

    def fetch(self, url):
        self.assertTrue(url.startswith(module.ORIGIN + "/"))
        path = url.removeprefix(module.ORIGIN)
        self.visited.append(path)
        return self.responses[path]

    def test_success_checks_pages_and_deduplicates_assets(self):
        result = module.verify(self.fetch, "0.2.51")
        self.assertEqual(result["assets"], 2)
        self.assertEqual(result["pages"], 3)
        self.assertEqual(self.visited.count("/app.js"), 1)
        self.assertIn("/sign-in", self.visited)
        self.assertNotIn("/login", self.visited)

    def test_checks_the_localized_legal_html_and_optionally_the_json_api(self):
        result = module.verify(self.fetch, check_legal_api=True)
        self.assertEqual(result["legal_documents"], 9)
        self.assertTrue(result["legal_api_checked"])
        self.assertIn("/api/privacy-policy?lang=zh-TW", self.visited)
        self.responses["/api/privacy-policy?lang=zh-TW"] = self.responses["/"]
        with self.assertRaisesRegex(ValueError, "public legal API"):
            module.verify(self.fetch, check_legal_api=True)

    def test_legal_documents_reject_personal_email_and_spa_fallback(self):
        self.responses["/user-agreement"] = module.Response(
            200, "text/html",
            b'<html lang="en"><article>personal&#64;example.net</article></html>',
        )
        with self.assertRaisesRegex(ValueError, "unexpected email domain"):
            module.verify(self.fetch)
        self.responses["/user-agreement"] = self.responses["/"]
        with self.assertRaisesRegex(ValueError, "missing en legal document"):
            module.verify(self.fetch)

    def test_public_sign_in_is_bound_to_the_selected_web_archive(self):
        expected = sha256(self.responses["/sign-in"].body).hexdigest()
        self.assertEqual(module.verify(self.fetch, expected_web_sha256=expected)["sign_in_sha256"], expected)
        with self.assertRaisesRegex(ValueError, "does not match the selected Web artifact"):
            module.verify(self.fetch, expected_web_sha256="0" * 64)
        with self.assertRaisesRegex(ValueError, "64 lowercase hexadecimal"):
            module.verify(self.fetch, expected_web_sha256="not-a-digest")

    def test_public_readiness_requires_a_ready_database(self):
        self.responses["/health/ready"] = module.Response(
            200, "application/json", b'{"status":"ready","database":false}'
        )
        with self.assertRaisesRegex(ValueError, "database is not ready"):
            module.verify(self.fetch)


    def test_private_release_identity_headers_are_rejected_on_public_status(self):
        self.responses["/api/status"] = module.Response(
            200, "application/json",
            json.dumps({"success": True, "data": {"version": "0.2.51"}}).encode(),
            {
                "X-Release-Commit": "a" * 40,
                "X-Core-Artifact-Sha256": "b" * 64,
                "X-Extensions-Artifact-Sha256": "c" * 64,
                "X-Schema-Fingerprint": "d" * 64,
                "X-Route-Identity": "edge-v2",
                "X-Request-Id": "req-123",
            },
        )
        with self.assertRaisesRegex(ValueError, "private release identity"):
            module.verify(self.fetch)

    def test_old_backend_is_not_accepted_as_success(self):
        with self.assertRaisesRegex(ValueError, "version mismatch"):
            module.verify(self.fetch, "0.2.52")

    def test_http_200_spa_fallback_is_not_a_successful_asset(self):
        self.responses["/app.js"] = self.responses["/"]
        with self.assertRaisesRegex(ValueError, "SPA fallback"):
            module.verify(self.fetch)

    def test_missing_stylesheet_fails(self):
        self.responses["/app.css"] = module.Response(404, "text/css", b"missing")
        with self.assertRaises(ValueError):
            module.verify(self.fetch)

    def test_service_error_disguised_as_html_fails(self):
        self.responses["/"] = module.Response(200, "text/html", b"service unavailable")
        with self.assertRaisesRegex(ValueError, "no local JavaScript"):
            module.verify(self.fetch)

    def test_false_api_success_fails(self):
        self.responses["/api/status"] = module.Response(200, "application/json", b'{"success":false,"data":{"version":"0.2.51"}}')
        with self.assertRaises(ValueError):
            module.verify(self.fetch)

    def test_cross_origin_redirect_is_refused(self):
        with self.assertRaisesRegex(ValueError, "cross-origin"):
            module.SameOriginRedirect().redirect_request(None, None, 302, "", {}, "https://untrusted.test/asset.js")

    def test_insecure_redirect_is_refused(self):
        with self.assertRaisesRegex(ValueError, "non-HTTPS"):
            module.SameOriginRedirect().redirect_request(None, None, 302, "", {}, "http://api.lmm.best/asset.js")


if __name__ == "__main__":
    unittest.main()
