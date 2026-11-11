Write todo.py, a command-line to-do list that uses only Python's standard library.

Tasks are stored as JSON in the file named by the environment variable TODO_FILE (default: todo.json in the current folder). The file may not exist yet. The layout of the JSON is up to you.

Commands:
- `python3 todo.py add "buy milk"` adds a task and prints `Added 1: buy milk`. Ids start at 1 and go up by one. An id is never reused, even after its task is removed.
- `python3 todo.py list` prints one line per task in id order: `1 [ ] buy milk`, or `1 [x] buy milk` once it is done. With no tasks it prints `No tasks.`
- `python3 todo.py done 1` marks task 1 done and prints `Done 1: buy milk`.
- `python3 todo.py remove 1` deletes task 1 and prints `Removed 1: buy milk`.
- For `done` and `remove`, if there is no task with that id, print `No task 1` (with the id given) to stderr and exit with code 1.

Successful commands exit with code 0 and print to stdout.
