import unittest
from decimal import Decimal

from benchmark import matches, rounded


class SoftComparisonTest(unittest.TestCase):
    def test_rounds_up_and_accepts_only_the_inclusive_point_six_margin(self):
        self.assertEqual(rounded(2.358490566037736), Decimal("2.4"))
        self.assertEqual(rounded(2.6000000000000001), Decimal("2.6"))
        self.assertTrue(matches(2.01, 2.61))
        self.assertTrue(matches(2.61, 2.01))
        self.assertFalse(matches(2.01, 2.71))
        self.assertFalse(matches(2.5, 25))
        self.assertTrue(matches(None, None))
        self.assertFalse(matches(None, 0))
        self.assertFalse(matches(0, None))
        self.assertFalse(matches(0, float("inf")))


if __name__ == "__main__":
    unittest.main()
