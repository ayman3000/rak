import json
import os
import sys

PATH = os.environ.get("TODO_FILE", "todo.json")


def load():
    try:
        with open(PATH) as f:
            return json.load(f)
    except FileNotFoundError:
        return {"next": 1, "tasks": []}


def save(data):
    with open(PATH, "w") as f:
        json.dump(data, f)


def main(argv):
    data = load()
    command = argv[0]
    if command == "add":
        task = {"id": data["next"], "text": argv[1], "done": False}
        data["next"] += 1
        data["tasks"].append(task)
        save(data)
        print(f"Added {task['id']}: {task['text']}")
    elif command == "list":
        if not data["tasks"]:
            print("No tasks.")
        for t in data["tasks"]:
            print(f"{t['id']} [{'x' if t['done'] else ' '}] {t['text']}")
    elif command in ("done", "remove"):
        wanted = int(argv[1])
        for t in data["tasks"]:
            if t["id"] == wanted:
                if command == "done":
                    t["done"] = True
                    print(f"Done {wanted}: {t['text']}")
                else:
                    data["tasks"].remove(t)
                    print(f"Removed {wanted}: {t['text']}")
                save(data)
                return 0
        print(f"No task {argv[1]}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
