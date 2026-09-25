#!/usr/bin/env python3
"""Attack-path discovery over the static cross-contract graph.

The analysis is intentionally conservative: it is designed to highlight risky
trust chains without mutating the onboard contract logic.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Dict, List, Set

from call_graph import build_call_graph, find_attack_paths, find_circular_dependencies

REPO_ROOT = Path(__file__).resolve().parents[2]


def severity_for_path(path: List[str]) -> str:
    if len(path) >= 5:
        return "high"
    if len(path) >= 3:
        return "medium"
    return "low"


def summarize() -> List[dict]:
    graph = build_call_graph()
    paths = find_attack_paths(graph)
    cycles = find_circular_dependencies(graph)
    results: List[dict] = []

    for path in paths:
        results.append(
            {
                "path": path,
                "severity": severity_for_path(path),
                "risk": "Cross-contract trust chain and state propagation pattern detected",
            }
        )

    for cycle in cycles:
        results.append(
            {
                "path": cycle,
                "severity": "high",
                "risk": "Circular dependency pattern detected across contracts",
            }
        )

    return sorted(results, key=lambda item: ("high".find(item["severity"]), len(item["path"])), reverse=True)


if __name__ == "__main__":
    print(json.dumps({"results": summarize()}, indent=2))
