import unittest
from legacy_money_features_import import plan


class Target:
    def run(self, sql):
        return "1" if "core_promotions.codes" in sql else "0"

    def rows(self, sql):
        if 'platform_role' in sql:
            return [{'id': 1}]
        if 'FROM core_identity.users' in sql:
            return [{'id': 1}]
        return []


class RedPacketImportTest(unittest.TestCase):
    def test_deleted_rewards_are_not_reported_missing_or_reactivated(self):
        data = {'packets': [], 'claims': [], 'discounts': [],
                'redemptions': [{'id': 7, 'deleted': True}],
                'items': [{'packet_id': 1, 'source_id': 7, 'item_type': 'redemption', 'claimed_by': 0},
                          {'packet_id': 1, 'source_id': 8, 'item_type': 'redemption', 'claimed_by': 0}]}
        sql, stats = plan(data, Target(), 100)
        self.assertEqual(stats['unclaimed_items_not_bound_redemption_deleted'], 1)
        self.assertEqual(stats['unclaimed_items_not_bound_redemption_missing'], 1)
        self.assertFalse(any('INSERT INTO core_promotions.red_packet_items' in s for s in sql))


if __name__ == '__main__':
    unittest.main()
