"""Check the upload boundary with synthetic private probe output."""
import runpy
import unittest
from pathlib import Path


probe_receipt = runpy.run_path(str(Path(__file__).with_name("extore-plugin-smoke.py")))["probe_receipt"]


class ProbeReceiptTests(unittest.TestCase):
    def setUp(self):
        self.host, self.plugin = "a" * 64, "b" * 64
        self.consumer = {
            "status": "passed", "host_abi": 1, "interface": "extore.commerce",
            "real_extore_https_sqlite": True, "native_digest_path_binding": True,
            "native_repeated_open_call_release_close": True, "store_sync_persisted": True,
            "http_catalog_fallback_calls": 0, "secret_values_logged": False,
            "production_accounts_created": False,
            "store_repository": "transactional memory test repository",
            "core_authority": "existing Store test double; independent CoreSession acceptance not run",
        }
        self.report = {
            "status": "passed", "consumer_exit_code": 0, "consumer": self.consumer,
            "host_library_sha256": self.host, "plugin_library_sha256": self.plugin,
            "actual_rust_health": True, "isolated_network_namespace": True,
            "tls_certificate_verification": True, "authorized_catalog": True,
            "cross_shop_http_detail_denied": True, "revoked_token_denied": True,
            "catalog_calls_did_not_issue_cards": True, "actual_sqlite_grant_usage_recorded": True,
        }

    def test_only_reviewed_fields_cross_the_boundary(self):
        private = dict(self.report, stdout="private fixture output",
                       consumer=dict(self.consumer, credentials={"test": "private"}))
        self.assertEqual(probe_receipt(private, self.host, self.plugin), self.report)

    def test_incomplete_failed_or_mistyped_facts_cannot_be_exported(self):
        missing = self.report.copy()
        del missing["consumer_exit_code"]
        for raw in [missing, dict(self.report, status="failed"), dict(self.report, consumer_exit_code=False),
                    dict(self.report, host_library_sha256=self.plugin),
                    dict(self.report, consumer=dict(self.consumer, core_authority="unreviewed text"))]:
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                probe_receipt(raw, self.host, self.plugin)


if __name__ == "__main__":
    unittest.main()
