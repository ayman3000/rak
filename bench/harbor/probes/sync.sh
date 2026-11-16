#!/bin/sh
# Copies the shared checker library into every probe's tests/ folder.
cd "$(dirname "$0")" || exit 1
for task in */; do
  case "$task" in _*) continue ;; esac
  cp _shared/checklib.py "$task/tests/checklib.py"
done
