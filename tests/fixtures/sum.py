import sys

values = list(map(int, sys.stdin.read().split()))
if len(values) != 2:
    raise SystemExit(2)
print(sum(values))

