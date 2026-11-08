import unittest

from smoke_ios import dedicated_device


class DedicatedSimulatorTests(unittest.TestCase):
    def test_only_explicit_available_dedicated_device_can_be_used(self):
        devices = {"runtime": [
            {"udid": "personal", "name": "My iPhone", "isAvailable": True},
            {"udid": "test", "name": "PeaZip-FirstRun", "isAvailable": True},
            {"udid": "missing", "name": "PeaZip-Old", "isAvailable": False},
        ]}
        self.assertEqual(dedicated_device(devices, "test")["runtime"], "runtime")
        for udid in ("personal", "missing", "unknown", "booted"):
            with self.assertRaises(ValueError):
                dedicated_device(devices, udid)


if __name__ == "__main__":
    unittest.main()
