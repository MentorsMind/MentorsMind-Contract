#!/usr/bin/env python3
"""Static cross-contract call graph analysis for the MentorsMind contract suite.

This utility does not modify existing contracts. It scans the Rust source tree,
extracts `env.invoke_contract`/`env.try_invoke_contract` edges from each contract,
and synthesizes a call graph plus a lightweight attack-path discovery report.
"""

from __future__ import annotations

import json
import re
from collections import defaultdict, deque
from pathlib import Path
from typing import Dict, Iterable, List, Set, Tuple

REPO_ROOT = Path(__file__).resolve().parents[2]
CONTRACT_ROOT = REPO_ROOT / "contracts"

CALL_RE = re.compile(
    r"env\.(?:try_)?invoke_contract\s*(?:<[^>]+>)?\s*\(\s*"
    r"(?P<target>[^,\n)]+?)\s*,\s*"
    r"(?:&\s*Symbol::new\s*\(\s*&?env\s*,\s*\"(?P<method>[^\"]+)\"\s*\)|"
    r"&\s*(?P<method_name>[A-Za-z0-9_]+)\s*|"
    r"(?P<method_expr>[A-Za-z0-9_]+)\s*\)\s*\))",
    re.MULTILINE,
)

SUSPICIOUS_METHODS = {
    "verify",
    "check_anomaly",
    "is_paused",
    "record_snapshot",
    "get_total_supply_at",
    "get_voting_power",
    "get_delegated_power_at_snapshot",
    "get_template_hash",
    "initialize",
    "create_escrow",
    "execute",
    "propose_admin_change",
    "is_verified",
    "restrict",
    "register_interface",
    "reconcile",
    "approve",
    "transfer",
    "transfer_from",
}


def normalize_target(raw: str) -> str:
    target = raw.strip()
    target = target.replace("&", "")
    target = target.replace(".clone()", "")
    target = target.replace(".to_val()", "")
    target = target.replace(".into_val(&env)", "")
    target = target.strip()
    return target


def contract_name_for_file(path: Path) -> str:
    return path.parent.parent.name


def analyze_contract_file(path: Path) -> List[dict]:
    text = path.read_text(encoding="utf-8", errors="ignore")
    contract_name = contract_name_for_file(path)
    edges: List[dict] = []

    for match in CALL_RE.finditer(text):
        target = normalize_target(match.group("target"))
        method = match.group("method") or match.group("method_name") or match.group("method_expr")
        if not method:
            continue
        if not target or target in {"env", "self", "&env", "&self", "&Address::generate", "Address::generate"}:
            continue
        edges.append(
            {
                "source": contract_name,
                "target": target,
                "method": method,
                "kind": "invoke_contract",
            }
        )
    return edges


def build_call_graph() -> Dict[str, Set[str]]:
    graph: Dict[str, Set[str]] = defaultdict(set)
    for rs_file in sorted(CONTRACT_ROOT.glob("*/src/*.rs")):
        for edge in analyze_contract_file(rs_file):
            graph[edge["source"]].add(f"{edge['target']}::{edge['method']}")
    return {key: sorted(value) for key, value in graph.items()}


def find_circular_dependencies(graph: Dict[str, Set[str]]) -> List[List[str]]:
    seen: Set[Tuple[str, ...]] = set()
    cycles: List[List[str]] = []

    for node in sorted(graph):
        queue: deque[Tuple[str, List[str]]] = deque([(node, [node])])
        while queue:
            current, path = queue.popleft()
            for neighbor in graph.get(current, []):
                target = neighbor.split("::", 1)[0]
                if target == node:
                    cycle = path + [node]
                    key = tuple(cycle)
                    if key not in seen:
                        seen.add(key)
                        cycles.append(cycle)
                    continue
                if target in path:
                    continue
                queue.append((target, path + [target]))
    return cycles


def find_attack_paths(graph: Dict[str, Set[str]], max_depth: int = 4) -> List[List[str]]:
    paths: List[List[str]] = []
    suspicious_nodes = {"guardian", "anomaly_detector", "registry", "snapshot_contract", "delegation_contract", "templates_contract", "implementation", "treasury"}

    for start in sorted(graph):
        queue: deque[Tuple[str, List[str]]] = deque([(start, [start])])
        while queue:
            current, path = queue.popleft()
            if len(path) >= max_depth:
                continue
            for edge in graph.get(current, []):
                target = edge.split("::", 1)[0]
                method = edge.split("::", 1)[1] if "::" in edge else ""
                next_path = path + [f"{target}:{method}"]
                if len(next_path) >= 2 and any(token in target.lower() for token in suspicious_nodes):
                    paths.append(next_path)
                if target not in {node.split(":", 1)[0] for node in path}:
                    queue.append((target, next_path))
    return paths


def render_mermaid(graph: Dict[str, Set[str]]) -> str:
    lines = ["graph TD"]
    for source, edges in sorted(graph.items()):
        for edge in edges:
            target, method = edge.split("::", 1) if "::" in edge else (edge, "call")
            lines.append(f'    {source} -->|{method}| {target}')
    return "\n".join(lines) + "\n"


def dump_report() -> dict:
    graph = build_call_graph()
    cycles = find_circular_dependencies(graph)
    attack_paths = find_attack_paths(graph)
    return {
        "contracts": sorted(graph),
        "edges": {key: sorted(value) for key, value in sorted(graph.items())},
        "circular_dependencies": cycles,
        "attack_paths": attack_paths,
        "mermaid": render_mermaid(graph),
    }


if __name__ == "__main__":
    report = dump_report()
    print(json.dumps(report, indent=2))
