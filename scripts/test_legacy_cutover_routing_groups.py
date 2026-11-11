"""Source group continuation preserves user grants and token restrictions."""
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

import financial_import as F
import legacy_cutover_import as I
from test_legacy_cutover_mapping import ARCHIVE, identity_files, source_data


class RoutingGroupTests(unittest.TestCase):
    def test_shared_identity_semantics_and_no_token_group_escalation(self):
        with tempfile.TemporaryDirectory(dir=F.IDENTITY.BACKUP_ROOT) as temporary:
            binding, _, _, _ = identity_files(Path(temporary))
            data = source_data(binding)
            data["options"]["UserUsableGroups"] = '{"default":"Default","paid":"Paid"}'
            data["users"][0]["group"] = " vip "
            data["users"][1]["group"] = ""
            data["tokens"][0].update(group="private-only", auto_groups='["ungranted"]')
            rows = [row for user in data["users"] for row in I.routing_group_rows(user, json.loads(data["options"]["UserUsableGroups"]))]
            self.assertEqual([(row["user_id"], row["group_name"]) for row in rows],
                             [(2, "default"), (2, "paid"), (2, "vip"), (3, "default"), (3, "paid")])
            self.assertEqual(rows[2]["source_reference"], "legacy:user_group")
            self.assertEqual(rows[3]["source_reference"], "legacy:UserUsableGroups")
            sql, stats = I.plan_routing_groups(data, ARCHIVE, "routing-phase")
            self.assertEqual(stats["grants"], 5)
            self.assertNotIn("private-only", "\n".join(sql))
            self.assertNotIn("ungranted", "\n".join(sql))
            for user in data["users"]:
                user.update(username="synthetic" + str(user["id"]), email="", password="", display_name="",
                            status=1, auth_version=1, created_at=0)
            data.update(two_fas=[], oauth_bindings=[], oauth_providers=[])
            original, _ = I.plan_identity(data, ARCHIVE)
            self.assertEqual([line for line in original if line.startswith("INSERT INTO core_identity.user_routing_groups")],
                             sql[:-1])
            self.assertNotIn("UPDATE core_identity", "\n".join(sql))

    def test_missing_formal_map_rejected_before_any_database_access(self):
        arguments = ["legacy_cutover_import.py", "--socket", "/tmp/synthetic", "--port", "65447",
                     "--source-db", "source", "--source-user", "reader", "--source-schema", "legacy",
                     "--target-db", "target", "--target-login", "installer", "--target-owner", "owner",
                     "--archive-sha256", ARCHIVE, "--migration-id", "routing-phase", "--observed-at", "1000",
                     "--phase", "routing_groups", "--output", "/tmp/not-created"]
        with patch.object(sys, "argv", arguments), patch.object(I, "Db") as database:
            with self.assertRaisesRegex(SystemExit, "committed_identity_map_and_manifest_required"):
                I.main()
            database.assert_not_called()

    def test_invalid_group_fails_instead_of_granting_auto_or_truncating(self):
        for usable in ([], {"auto": "Auto"}, {"x" * 65: "Too long"}, {"": "Empty"}):
            with self.subTest(usable=usable), self.assertRaises(SystemExit):
                I.routing_group_rows(dict(id=2, group="default"), usable)


if __name__ == "__main__":
    unittest.main()
