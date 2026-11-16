import subprocess
import sys

from checklib import check, finish

sys.path.insert(0, "/app/pager")


def visible_tests():
    done = subprocess.run([sys.executable, "-m", "pytest", "-q"], cwd="/app/pager", capture_output=True, text=True)
    assert done.returncode == 0, done.stdout[-500:]
    return True


def tests_unchanged():
    return open("/app/pager/tests/test_pager.py").read() == open("/tests/test_pager_original.py").read()


def hidden():
    from pager import paginate

    assert paginate(list("abcdefg"), 1, 3) == ["a", "b", "c"]
    assert paginate(list("abcdefg"), 3, 3) == ["g"]
    assert paginate(list("abcdefg"), 9, 3) == []
    assert paginate([], 1, 5) == []
    return True


check("the tests pass", visible_tests)
check("the tests are unchanged", tests_unchanged)
check("hidden cases", hidden)
finish()
