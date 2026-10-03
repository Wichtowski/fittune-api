import time
import unittest

from service import Worker


def stalled(connection):
    connection.send({"ready": True})
    connection.recv_bytes()
    time.sleep(60)


class WatchdogTest(unittest.TestCase):
    def test_timeout_kills_inference_and_restarts_worker(self):
        worker = Worker(stalled)
        try:
            self.assertTrue(worker.connection.poll(5))
            self.assertTrue(worker.health())
            previous = worker.process
            with self.assertRaisesRegex(RuntimeError, "timed out"):
                worker.run(b"test", timeout=0.05)
            self.assertFalse(previous.is_alive())
            self.assertNotEqual(previous.pid, worker.process.pid)
            self.assertTrue(worker.connection.poll(5))
            self.assertTrue(worker.health())
            self.assertTrue(worker.lock.acquire(blocking=False))
            worker.lock.release()
        finally:
            worker.stop()
