#!/usr/bin/env python3
"""Collect `coding-tooling/work-evidence/v1` for the operations in .performance/contract.json.

The `performance:work` capability: runs the blocking performance smoke, which measures every
declared operation at every declared scale point and checks the declared bounds, then adds
the suite and the sha256 of the exact contract bytes. The evidence goes to stdout, and also
to --output when given.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONTRACT = ROOT / ".performance" / "contract.json"
SCHEMA_VERSION = "coding-tooling/work-evidence/v1"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, help="also write the evidence to this file")
    arguments = parser.parse_args()

    contract_bytes = CONTRACT.read_bytes()
    contract = json.loads(contract_bytes)
    smoke = subprocess.run(
        [
            "cargo",
            "run",
            "--locked",
            "--release",
            "--quiet",
            "-p",
            "arpg-core",
            "--features",
            "operation-work",
            "--example",
            "performance_smoke",
        ],
        cwd=ROOT,
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    lines = smoke.stdout.strip().splitlines()
    if not lines:
        raise RuntimeError("performance smoke wrote no work evidence")
    measured = json.loads(lines[-1])
    if set(measured) != {"operations"}:
        raise RuntimeError("performance smoke did not end with the operations evidence line")

    evidence = {
        "schemaVersion": SCHEMA_VERSION,
        "suite": contract["suite"],
        "contractSha256": hashlib.sha256(contract_bytes).hexdigest(),
        "operations": measured["operations"],
    }
    payload = json.dumps(evidence, indent=2, sort_keys=True) + "\n"
    if arguments.output is not None:
        arguments.output.parent.mkdir(parents=True, exist_ok=True)
        arguments.output.write_text(payload, encoding="utf-8")
    sys.stdout.write(payload)


if __name__ == "__main__":
    main()
