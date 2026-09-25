#!/usr/bin/env python3
"""Render a human-readable Mermaid security diagram for cross-contract edges."""

from __future__ import annotations

from call_graph import build_call_graph, render_mermaid

if __name__ == "__main__":
    graph = build_call_graph()
    print(render_mermaid(graph))
