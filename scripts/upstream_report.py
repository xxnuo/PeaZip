"""Report upstream changes without advancing the reviewed baseline."""

import argparse
import json
from pathlib import Path
import subprocess


ROOT = Path(__file__).resolve().parents[1]


def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT, text=True).strip()


def report(target=None):
    baseline = json.loads((ROOT / "upstream-baseline.json").read_text())["peazip"]
    start = git("rev-parse", "--verify", baseline["revision"] + "^{commit}")
    end = git("rev-parse", "--verify", "--end-of-options", (target or baseline["ref"]) + "^{commit}")
    subprocess.run(["git", "merge-base", "--is-ancestor", start, end], cwd=ROOT, check=True)
    return {
        "baseline": start,
        "target": end,
        "commits": git("log", "--format=%H %s", f"{start}..{end}").splitlines(),
        "files": git("diff", "--name-status", start, end).splitlines(),
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", help="Fetched upstream ref or commit")
    args = parser.parse_args()
    try:
        print(json.dumps(report(args.target), ensure_ascii=False, indent=2))
    except (subprocess.CalledProcessError, OSError, ValueError, KeyError) as error:
        parser.exit(1, f"Upstream report failed: {error}\n")
