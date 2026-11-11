import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))


class Hidden(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp()
        self.env = dict(os.environ, TODO_FILE=os.path.join(self.dir, "tasks.json"))

    def todo(self, *args):
        return subprocess.run([sys.executable, os.path.join(HERE, "todo.py"), *args],
                              capture_output=True, text=True, env=self.env, cwd=self.dir, timeout=20)

    def test_empty_list(self):
        r = self.todo("list")
        self.assertEqual((r.returncode, r.stdout.strip()), (0, "No tasks."))

    def test_add_and_list(self):
        self.assertEqual(self.todo("add", "buy milk").stdout.strip(), "Added 1: buy milk")
        self.assertEqual(self.todo("add", "call mum").stdout.strip(), "Added 2: call mum")
        self.assertEqual(self.todo("list").stdout.strip().splitlines(), ["1 [ ] buy milk", "2 [ ] call mum"])

    def test_done(self):
        self.todo("add", "buy milk")
        self.todo("add", "call mum")
        r = self.todo("done", "2")
        self.assertEqual((r.returncode, r.stdout.strip()), (0, "Done 2: call mum"))
        self.assertEqual(self.todo("list").stdout.strip().splitlines(), ["1 [ ] buy milk", "2 [x] call mum"])

    def test_remove_and_ids_are_not_reused(self):
        self.todo("add", "a")
        self.todo("add", "b")
        self.assertEqual(self.todo("remove", "2").stdout.strip(), "Removed 2: b")
        self.assertEqual(self.todo("add", "c").stdout.strip(), "Added 3: c")
        self.assertEqual(self.todo("list").stdout.strip().splitlines(), ["1 [ ] a", "3 [ ] c"])

    def test_removing_everything(self):
        self.todo("add", "a")
        self.todo("remove", "1")
        self.assertEqual(self.todo("list").stdout.strip(), "No tasks.")
        self.assertEqual(self.todo("add", "b").stdout.strip(), "Added 2: b")

    def test_unknown_id(self):
        self.todo("add", "a")
        for command in ("done", "remove"):
            r = self.todo(command, "7")
            self.assertEqual(r.returncode, 1)
            self.assertEqual(r.stderr.strip(), "No task 7")
            self.assertEqual(r.stdout.strip(), "")

    def test_uses_todo_file(self):
        self.todo("add", "a")
        self.assertTrue(os.path.exists(self.env["TODO_FILE"]))
