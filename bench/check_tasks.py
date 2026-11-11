"""Checks the benchmark itself: for every task the starting files must fail
the hidden tests and the reference solution must pass them."""
import pathlib, shutil, subprocess, sys, tempfile

root = pathlib.Path(__file__).parent / "tasks"
bad = 0
for task in sorted(p for p in root.iterdir() if p.is_dir()):
    results = []
    for with_solution in (False, True):
        work = pathlib.Path(tempfile.mkdtemp())
        shutil.copytree(task / "start", work, dirs_exist_ok=True)
        if with_solution:
            shutil.copytree(task / "solution", work, dirs_exist_ok=True)
        shutil.copytree(task / "hidden", work, dirs_exist_ok=True)
        r = subprocess.run([sys.executable, "-m", "unittest", "discover", "-s", str(work), "-p", "hidden_test*.py"],
                           capture_output=True, text=True, cwd=work)
        results.append(r.returncode == 0)
        if with_solution and r.returncode != 0:
            print(r.stderr[-1500:])
    ok = results == [False, True]
    bad += not ok
    print(f"{'ok ' if ok else 'BAD'} {task.name}: start passes={results[0]} solution passes={results[1]}")
sys.exit(1 if bad else 0)
